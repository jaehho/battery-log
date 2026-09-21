//! Low-battery notifications.
//!
//! This decides nothing about the battery. UPower already turns the percentage
//! into a `WarningLevel` using the thresholds in `UPower.conf`, and those same
//! thresholds drive its critical action (hibernate). Showing that level,
//! instead of keeping a second set of numbers here, means the warning and the
//! action can never disagree, and it inherits UPower's handling of multiple
//! packs and of USB-C supplies that misreport `online`.
//!
//! The notification is a view of the level, not a stream of events: one slot,
//! updated in place when the level rises, closed when it clears. Nothing here
//! ever suspends or hibernates the machine.
//!
//! [`decide`] is the whole policy and touches no bus; [`watch`] is the wiring.

use std::thread;

/// UPower's `WarningLevel`, collapsed to what a person needs to see.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Unknown, None, or Discharging (the last only applies to UPS devices).
    Clear,
    Low,
    Critical,
    /// The critical action is about to run.
    Action,
}

impl Level {
    pub fn from_upower(v: u32) -> Level {
        match v {
            3 => Level::Low,
            4 => Level::Critical,
            5 => Level::Action,
            _ => Level::Clear,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Reading {
    pub level: Level,
    pub percent: f64,
    /// Seconds; 0 when UPower has no estimate yet.
    pub time_to_empty: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    /// freedesktop urgency: 1 normal, 2 critical.
    pub urgency: u8,
    pub icon: &'static str,
    pub summary: String,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Show(Note),
    Close,
    Nothing,
}

/// What to do with the notification, given the level it currently shows.
///
/// It only speaks on a change of level. Re-showing on every percent would
/// re-pop the notification each minute, and a person who dismissed "low"
/// does not need to be told again until it becomes "critical".
pub fn decide(shown: Level, now: &Reading) -> Op {
    if now.level == shown {
        return Op::Nothing;
    }
    if now.level == Level::Clear {
        return Op::Close;
    }
    let pct = now.percent.round() as i64;
    let left = remaining(now.time_to_empty);
    let note = match now.level {
        Level::Low => Note {
            urgency: 1,
            icon: "battery-low",
            summary: "Battery low".into(),
            body: format!("{pct}%{left}"),
        },
        Level::Critical => Note {
            urgency: 2,
            icon: "battery-caution",
            summary: "Battery critical".into(),
            body: format!("{pct}%{left}. Plug in soon."),
        },
        Level::Action => Note {
            urgency: 2,
            icon: "battery-empty",
            summary: "Hibernating".into(),
            body: format!("{pct}%. Plug in now to stay awake."),
        },
        Level::Clear => unreachable!(),
    };
    Op::Show(note)
}

fn remaining(secs: i64) -> String {
    if secs <= 0 {
        return String::new();
    }
    let m = (secs + 30) / 60;
    if m >= 60 {
        format!(", about {}h {:02}m left", m / 60, m % 60)
    } else {
        format!(", about {m}m left")
    }
}

const UPOWER: &str = "org.freedesktop.UPower";
const DISPLAY: &str = "/org/freedesktop/UPower/devices/DisplayDevice";
const DEVICE: &str = "org.freedesktop.UPower.Device";

fn get<T>(conn: &zbus::blocking::Connection, prop: &str) -> Option<T>
where
    T: TryFrom<zbus::zvariant::OwnedValue>,
{
    let msg = conn
        .call_method(
            Some(UPOWER),
            DISPLAY,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(DEVICE, prop),
        )
        .ok()?;
    let v: zbus::zvariant::OwnedValue = msg.body().deserialize().ok()?;
    T::try_from(v).ok()
}

fn read(conn: &zbus::blocking::Connection) -> Option<Reading> {
    Some(Reading {
        level: Level::from_upower(get::<u32>(conn, "WarningLevel")?),
        percent: get::<f64>(conn, "Percentage").unwrap_or(0.0),
        time_to_empty: get::<i64>(conn, "TimeToEmpty").unwrap_or(0),
    })
}

/// The session side: one notification slot, reconnecting when the notification
/// daemon was not up yet (the service starts before the compositor does).
#[derive(Default)]
struct Notifier {
    conn: Option<zbus::blocking::Connection>,
    id: u32,
}

impl Notifier {
    fn bus(&mut self) -> Option<&zbus::blocking::Connection> {
        if self.conn.is_none() {
            self.conn = zbus::blocking::Connection::session().ok();
        }
        self.conn.as_ref()
    }

    /// True when the op landed; on failure the caller keeps its old state and
    /// the next UPower change tries again.
    fn apply(&mut self, op: &Op) -> bool {
        let id = self.id;
        let Some(conn) = self.bus() else { return false };
        let result = match op {
            Op::Nothing => return true,
            Op::Close => {
                if id == 0 {
                    return true;
                }
                conn.call_method(
                    Some("org.freedesktop.Notifications"),
                    "/org/freedesktop/Notifications",
                    Some("org.freedesktop.Notifications"),
                    "CloseNotification",
                    &(id,),
                )
                .map(|_| 0)
            }
            Op::Show(n) => {
                let mut hints = std::collections::HashMap::new();
                hints.insert("urgency", zbus::zvariant::Value::U8(n.urgency));
                conn.call_method(
                    Some("org.freedesktop.Notifications"),
                    "/org/freedesktop/Notifications",
                    Some("org.freedesktop.Notifications"),
                    "Notify",
                    &(
                        "battery-log",
                        id,
                        n.icon,
                        n.summary.as_str(),
                        n.body.as_str(),
                        Vec::<&str>::new(),
                        hints,
                        -1i32,
                    ),
                )
                .and_then(|m| m.body().deserialize::<u32>())
            }
        };
        match result {
            Ok(new_id) => {
                self.id = new_id;
                true
            }
            Err(e) => {
                eprintln!("battery-log: notification failed ({e}); will retry");
                self.conn = None;
                false
            }
        }
    }
}

/// Follow UPower's display device and keep the notification in step with it.
/// Runs on its own thread; a missing bus or UPower only costs the warnings.
pub fn watch() {
    thread::spawn(|| {
        let conn = match zbus::blocking::Connection::system() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("battery-log: no system bus ({e}); no battery warnings");
                return;
            }
        };
        let props = match zbus::blocking::fdo::PropertiesProxy::builder(&conn)
            .destination(UPOWER)
            .and_then(|b| b.path(DISPLAY))
            .and_then(|b| b.build())
        {
            Ok(p) => p,
            Err(e) => {
                eprintln!("battery-log: cannot reach UPower ({e}); no battery warnings");
                return;
            }
        };
        let changes = match props.receive_properties_changed() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("battery-log: cannot follow UPower ({e}); no battery warnings");
                return;
            }
        };

        let mut notifier = Notifier::default();
        let mut shown = Level::Clear;
        // Reconcile once at start, so a daemon started at 8% says so, then on
        // every change UPower reports.
        let step = |notifier: &mut Notifier, shown: &mut Level| {
            if let Some(now) = read(&conn) {
                if notifier.apply(&decide(*shown, &now)) {
                    *shown = now.level;
                }
            }
        };
        step(&mut notifier, &mut shown);
        for _ in changes {
            step(&mut notifier, &mut shown);
        }
    });
}

/// Against the real buses: `cargo test -p battery-log-core notify -- --ignored`.
/// Reads UPower, then walks a notification through low -> critical -> close.
#[cfg(test)]
mod live {
    use super::*;

    #[test]
    #[ignore]
    fn probe() {
        let sys = zbus::blocking::Connection::system().unwrap();
        let now = read(&sys).expect("UPower display device");
        eprintln!("UPower now: {now:?}");

        let mut n = Notifier::default();
        let low = Reading {
            level: Level::Low,
            percent: 19.0,
            time_to_empty: 4000,
        };
        assert!(n.apply(&decide(Level::Clear, &low)));
        let first = n.id;
        std::thread::sleep(std::time::Duration::from_secs(2));
        let crit = Reading {
            level: Level::Critical,
            percent: 9.0,
            time_to_empty: 0,
        };
        assert!(n.apply(&decide(Level::Low, &crit)));
        assert_eq!(n.id, first, "escalation must reuse the slot");
        std::thread::sleep(std::time::Duration::from_secs(2));
        assert!(n.apply(&Op::Close));
    }
}
