//! The sampler.
//!
//! A daemon rather than a systemd timer for two reasons a timer cannot cover:
//! it listens for logind's `PrepareForSleep` so a sample lands *at* the suspend
//! boundary, and it samples faster while on battery than on wall power.

use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crate::model::Kind;
use crate::procs::ProcWatcher;
use crate::store::{Store, RETENTION_DAYS};
use crate::sysfs::Reader;

pub struct Config {
    /// Seconds between samples while discharging.
    pub on_battery_s: u64,
    /// Seconds between samples on wall power, where nothing interesting moves.
    pub on_ac_s: u64,
    /// How many processes to record (0 disables it).
    pub top_procs: usize,
    /// Minimum seconds between process snapshots. The sample interval on
    /// battery is deliberately short; process rows do not need to be.
    pub procs_every_s: u64,
    /// Mirror UPower's warning level as a desktop notification.
    pub notify: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            on_battery_s: 20,
            on_ac_s: 60,
            top_procs: 5,
            procs_every_s: 60,
            notify: true,
        }
    }
}

/// A logind delay inhibitor. Holding it is what stops the machine suspending
/// between the `PrepareForSleep` signal and our write; releasing it is how the
/// suspend is allowed to proceed, so the lock exists purely for its lifetime.
struct SleepLock {
    bus: Option<zbus::blocking::Connection>,
    fd: Option<zbus::zvariant::OwnedFd>,
}

impl SleepLock {
    fn new() -> Self {
        let mut lock = SleepLock {
            bus: zbus::blocking::Connection::system().ok(),
            fd: None,
        };
        lock.acquire();
        if lock.fd.is_none() {
            eprintln!("battery-log: no sleep inhibitor; a suspend may race the last sample");
        }
        lock
    }

    fn acquire(&mut self) {
        if self.fd.is_none() {
            self.fd = self.bus.as_ref().and_then(take_delay_lock);
        }
    }

    fn release(&mut self) {
        self.fd = None;
    }
}

fn take_delay_lock(conn: &zbus::blocking::Connection) -> Option<zbus::zvariant::OwnedFd> {
    let reply = conn
        .call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1",
            Some("org.freedesktop.login1.Manager"),
            "Inhibit",
            &(
                "sleep",
                "battery-log",
                "recording a sample at the suspend boundary",
                "delay",
            ),
        )
        .ok()?;
    reply.body().deserialize::<zbus::zvariant::OwnedFd>().ok()
}

/// Forward `PrepareForSleep(bool)` onto a channel: true just before suspend,
/// false just after resume.
fn watch_sleep(tx: Sender<bool>) {
    thread::spawn(move || {
        let conn = match zbus::blocking::Connection::system() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("battery-log: no system bus ({e}); suspend edges will be inferred");
                return;
            }
        };
        let proxy = match zbus::blocking::Proxy::new(
            &conn,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        ) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("battery-log: cannot reach logind ({e})");
                return;
            }
        };
        let iter = match proxy.receive_signal("PrepareForSleep") {
            Ok(i) => i,
            Err(e) => {
                eprintln!("battery-log: cannot subscribe to PrepareForSleep ({e})");
                return;
            }
        };
        for msg in iter {
            if let Ok(going_down) = msg.body().deserialize::<bool>() {
                let _ = tx.send(going_down);
            }
        }
    });
}

pub fn run(cfg: Config) -> Result<(), Box<dyn std::error::Error>> {
    if cfg.notify {
        crate::notify::watch();
    }
    let store = Store::open()?;
    let mut reader = Reader::new();
    let mut procs = ProcWatcher::default();
    let (tx, rx): (Sender<bool>, Receiver<bool>) = channel();
    watch_sleep(tx);

    // Held whenever we are awake, released only for the suspend we were told
    // about.
    let mut lock = SleepLock::new();

    let mut kind = Kind::Start;
    let mut last_prune = Instant::now();
    let mut last_procs: Option<Instant> = None;
    loop {
        let sample = reader.sample(kind);
        let mut wait = Duration::from_secs(cfg.on_ac_s);
        if let Some(s) = sample {
            let due = last_procs
                .map(|t| t.elapsed() >= Duration::from_secs(cfg.procs_every_s))
                .unwrap_or(true);
            let mut top = Vec::new();
            if due && cfg.top_procs > 0 {
                last_procs = Some(Instant::now());
                // The counters are read even on AC, or the first sample after
                // unplugging would credit one process with hours of CPU.
                let ranked = procs.top(cfg.top_procs);
                if s.status == "Discharging" {
                    top = ranked;
                }
            }
            wait = Duration::from_secs(if s.status == "Discharging" {
                cfg.on_battery_s
            } else {
                cfg.on_ac_s
            });
            if let Err(e) = store.insert(&s, &top) {
                eprintln!("battery-log: insert failed: {e}");
            }
        } else if kind == Kind::Start {
            // No battery at all: a desktop, or a pack the kernel has not
            // enumerated. Nothing to do, and nothing to fail about.
            eprintln!("battery-log: no system battery; not logging");
            return Ok(());
        }

        if last_prune.elapsed() > Duration::from_secs(6 * 3600) {
            let _ = store.prune(RETENTION_DAYS);
            last_prune = Instant::now();
        }

        kind = Kind::Tick;
        match rx.recv_timeout(wait) {
            Ok(true) => {
                // Going to sleep: sample now, then release the lock so the
                // suspend can proceed.
                if let Some(s) = reader.sample(Kind::PreSleep) {
                    let _ = store.insert(&s, &[]);
                }
                lock.release();
                // Wait for the matching resume rather than spinning.
                while let Ok(going_down) = rx.recv() {
                    if !going_down {
                        break;
                    }
                }
                lock.acquire();
                kind = Kind::Resume;
            }
            Ok(false) => kind = Kind::Resume,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                thread::sleep(wait);
            }
        }
    }
}
