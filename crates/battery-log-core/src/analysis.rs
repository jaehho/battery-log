//! Turning rows into the numbers the dashboard shows.
//!
//! Everything is an attribute of the *interval between* two samples, not of a
//! sample: energy drained, time on battery, time asleep. The daemon writes a
//! sample on either side of a suspend, so a sleep is usually one clean
//! interval; where it is not (an old row, a crash, a hard power-off) the
//! `slept_s` field still says how much of the interval was suspended.

use std::collections::BTreeMap;

use chrono::{Local, NaiveDate, TimeZone, Timelike};
use serde::Serialize;

use crate::model::{ProcDelta, Sample};
use crate::store::Store;

/// Where the daemon measured the suspend directly, anything over a minute is a
/// real sleep.
const MEASURED_SLEEP_S: i64 = 60;
/// Where it did not -- old rows, a crash, a killed daemon -- a sleep has to be
/// inferred from the hole, and a hole this small is as likely to be a missed
/// sample as a suspend. Guessing on those is what produced 20 W "standby".
const INFERRED_GAP_S: i64 = 600;

#[derive(Debug, Default, Clone, Serialize)]
pub struct DayAgg {
    pub date: String,
    pub awake_h: f64,
    pub battery_h: f64,
    pub asleep_h: f64,
    pub off_h: f64,
    /// Time inside the day that no interval could be attributed to anything --
    /// the sampler was down, or the rows on either side are too coarse to read.
    pub unaccounted_h: f64,
    pub drained_wh: f64,
    pub charged_wh: f64,
    /// Wh lost across the sleeps whose drain was actually measurable, and the
    /// hours of *those* sleeps. Sleeps spent on AC are not divided into.
    pub standby_wh: f64,
    pub standby_h: f64,
    pub avg_w: Option<f64>,
    pub standby_w: Option<f64>,
    /// The day leans on upower's seeded rows, which only recorded a sample when
    /// the percentage changed. Its numbers are an estimate, not a measurement.
    pub estimated: bool,
    pub min_cap: Option<i64>,
    pub max_cap: Option<i64>,
    pub full_wh: Option<f64>,
    pub cycles: Option<i64>,
    /// Mean backlight while on battery, 0-100.
    pub backlight: Option<f64>,
    /// Mean CPU busy while on battery.
    pub cpu_busy: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Sleep {
    pub start: i64,
    pub end: i64,
    pub hours: f64,
    /// `None` when the drain is not knowable -- the machine was on AC, or the
    /// pack recalibrated mid-sleep. Zero would be a claim; this is not one.
    pub drained_wh: Option<f64>,
    pub watts: Option<f64>,
    pub cap_from: Option<i64>,
    pub cap_to: Option<i64>,
    /// Share of the sleep the platform spent in real hardware sleep. `None`
    /// when the kernel does not report it; near zero means s2idle stayed awake.
    pub hw_residency: Option<f64>,
    /// The machine was plugged in for some of this, so nothing about standby
    /// drain can be read off it.
    pub on_ac: bool,
    /// The machine was off, not suspended (a different boot).
    pub powered_off: bool,
    /// Both ends were recorded at the suspend boundary, so the numbers are exact.
    pub exact: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Bucket {
    pub label: String,
    pub avg_w: Option<f64>,
    pub hours: f64,
    pub wh: f64,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct Curve {
    pub t: Vec<f64>,
    pub cap: Vec<Option<i64>>,
    pub w: Vec<Option<f64>>,
    pub cpu: Vec<Option<f64>>,
    /// 0 idle, 1 on battery, 2 charging, 3 on AC
    pub state: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BatteryNow {
    pub model: String,
    pub capacity: Option<i64>,
    pub status: String,
    pub power_w: Option<f64>,
    pub profile: String,
    pub full_wh: Option<f64>,
    pub design_wh: Option<f64>,
    pub health: Option<f64>,
    pub cycles: Option<i64>,
    pub runtime_h: Option<f64>,
    pub backlight: Option<f64>,
    pub displays_on: i64,
    pub external_displays: i64,
    pub cpu_busy: Option<f64>,
    pub temp_c: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogInfo {
    pub samples: i64,
    pub since: Option<String>,
    pub bytes: u64,
    pub seeded: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Dashboard {
    pub generated: i64,
    pub range_days: i64,
    pub battery: Option<BatteryNow>,
    pub days: Vec<DayAgg>,
    pub curve: BTreeMap<String, Curve>,
    pub sleeps: Vec<Sleep>,
    pub by_hour: Vec<Bucket>,
    pub by_profile: Vec<Bucket>,
    pub by_display: Vec<Bucket>,
    pub by_backlight: Vec<Bucket>,
    pub by_cpu: Vec<Bucket>,
    pub top_procs: Vec<ProcDelta>,
    pub log: LogInfo,
}

/// The local calendar day a timestamp falls in.
pub fn local_day(ts: i64) -> String {
    day_of(ts)
}

fn day_of(ts: i64) -> String {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

fn minutes_of(ts: i64) -> f64 {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|d| d.hour() as f64 * 60.0 + d.minute() as f64 + d.second() as f64 / 60.0)
        .unwrap_or(0.0)
}

pub fn day_start(date: &str) -> i64 {
    NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .and_then(|dt| Local.from_local_datetime(&dt).single())
        .map(|dt| dt.timestamp())
        .unwrap_or(0)
}

/// Energy drawn from the pack between two samples, in Wh. Positive is drain.
///
/// `energy_now` is not a fixed ruler. The firmware re-estimates `energy_full`
/// as it learns the pack -- this one swings between 64.3 and 70.3 Wh, nearly
/// 9% -- and rebases `energy_now` along with it, so subtracting two raw
/// readings across a recalibration invents watt-hours that never flowed. When
/// the ruler moves, difference the state of charge instead and price it at the
/// newer capacity. Over one 16-hour suspend that is the difference between
/// "0.00 W" and the real 0.11 W.
fn drained_wh(prev: &Sample, cur: &Sample) -> Option<f64> {
    let (pe, ce) = (prev.energy_uwh?, cur.energy_uwh?);
    match (prev.full_uwh, cur.full_uwh) {
        (Some(pf), Some(cf)) if pf != cf && pf > 0 && cf > 0 => {
            let soc = pe as f64 / pf as f64 - ce as f64 / cf as f64;
            Some(soc * cf as f64 / 1e6)
        }
        _ => Some((pe - ce) as f64 / 1e6),
    }
}

/// Share of a suspend the platform spent in real hardware sleep.
///
/// Prefer the kernel's per-suspend `last_hw_sleep`, read at resume. The
/// running `total_hw_sleep` is the fallback, and it is only a fallback: on
/// this firmware it has been seen to advance by exactly 2^32 us across a
/// 71-minute suspend. Anything that lands outside 0..1 is that bug, not a
/// measurement, so it is dropped rather than clamped into looking plausible.
fn hw_residency(prev: &Sample, cur: &Sample, slept_s: i64) -> Option<f64> {
    if slept_s <= 0 {
        return None;
    }
    let window = slept_s as f64 * 1e6;
    let per_suspend = match cur.last_hw_sleep_us {
        Some(us) if cur.kind == "resume" && us >= 0 => Some(us as f64),
        _ => None,
    };
    let from_total = match (prev.hw_sleep_us, cur.hw_sleep_us) {
        (Some(a), Some(b)) if b >= a => Some((b - a) as f64),
        _ => None,
    };
    let us = per_suspend.or(from_total)?;
    // Hardware sleep is a subset of the suspend -- freeze and thaw are not in
    // it -- so it can never genuinely exceed `slept_s`. Allow two seconds for
    // the two clocks rounding against each other and reject the rest.
    (us >= 0.0 && us <= window + 2e6).then(|| round2((us / window).min(1.0)))
}

/// Split an interval into the local days it spans, as (day, hours).
///
/// A suspend that starts on Sunday night and ends on Monday afternoon belongs
/// to both. Charging all sixteen hours to Sunday is how one day in the log
/// came to be thirty-two hours long.
fn per_day(t0: i64, t1: i64) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    let mut t = t0;
    while t < t1 && out.len() < 400 {
        // Midnight after `t`. Adding 86_400 would drift across a daylight-saving
        // change, so ask the calendar instead.
        let next = Local
            .timestamp_opt(t, 0)
            .single()
            .and_then(|d| d.date_naive().succ_opt())
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .and_then(|dt| Local.from_local_datetime(&dt).earliest())
            .map(|dt| dt.timestamp())
            .filter(|&n| n > t)
            .unwrap_or(t1);
        let end = t1.min(next);
        out.push((day_of(t), (end - t) as f64 / 3600.0));
        t = end;
    }
    out
}

#[derive(Default)]
struct Accum {
    wh: f64,
    hours: f64,
}

impl Accum {
    fn add(&mut self, wh: f64, hours: f64) {
        self.wh += wh;
        self.hours += hours;
    }
    fn bucket(&self, label: &str) -> Bucket {
        Bucket {
            label: label.to_string(),
            avg_w: if self.hours > 0.02 {
                Some(round2(self.wh / self.hours))
            } else {
                None
            },
            hours: round2(self.hours),
            wh: round2(self.wh),
        }
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn backlight_label(v: f64) -> &'static str {
    match v {
        v if v <= 25.0 => "0-25%",
        v if v <= 50.0 => "26-50%",
        v if v <= 75.0 => "51-75%",
        _ => "76-100%",
    }
}

fn cpu_label(v: f64) -> &'static str {
    match v {
        v if v < 5.0 => "idle (<5%)",
        v if v < 20.0 => "light (5-20%)",
        v if v < 50.0 => "busy (20-50%)",
        _ => "loaded (>50%)",
    }
}

fn display_label(on: i64, external: i64) -> String {
    if on == 0 {
        "screen off".to_string()
    } else if external == 0 {
        "laptop panel".to_string()
    } else if external == 1 {
        "+1 external".to_string()
    } else {
        format!("+{external} external")
    }
}

/// Walk the samples once and produce everything the dashboard needs.
pub fn analyse(
    rows: &[Sample],
) -> (
    Vec<DayAgg>,
    Vec<Sleep>,
    Vec<Bucket>,
    Vec<Bucket>,
    Vec<Bucket>,
    Vec<Bucket>,
    Vec<Bucket>,
) {
    let mut days: BTreeMap<String, DayAgg> = BTreeMap::new();
    let mut sleeps: Vec<Sleep> = Vec::new();
    let mut by_hour: BTreeMap<u32, Accum> = BTreeMap::new();
    let mut by_profile: BTreeMap<String, Accum> = BTreeMap::new();
    let mut by_display: BTreeMap<String, Accum> = BTreeMap::new();
    let mut by_backlight: BTreeMap<String, Accum> = BTreeMap::new();
    let mut by_cpu: BTreeMap<String, Accum> = BTreeMap::new();
    // Weighted means for the per-day context columns.
    let mut day_ctx: BTreeMap<String, (f64, f64, f64, f64)> = BTreeMap::new();
    // Every full-charge reading of the day. The firmware re-estimates this
    // constantly, so the max is the top of the noise, not the capacity --
    // take the median.
    let mut day_full: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    // First and last sample in each day, so time we could not attribute is
    // visible rather than silently missing.
    let mut day_span: BTreeMap<String, (i64, i64)> = BTreeMap::new();

    for (i, cur) in rows.iter().enumerate() {
        let d = days.entry(day_of(cur.ts)).or_insert_with(|| DayAgg {
            date: day_of(cur.ts),
            ..Default::default()
        });
        if let Some(c) = cur.capacity {
            d.min_cap = Some(d.min_cap.map_or(c, |m: i64| m.min(c)));
            d.max_cap = Some(d.max_cap.map_or(c, |m: i64| m.max(c)));
        }
        // Health only from sysfs rows: seeded upower rows carry today's
        // energy_full stamped on every historical sample, which would draw a
        // flat -- and false -- degradation line.
        if cur.source == "sysfs" {
            if let Some(f) = cur.full_uwh {
                day_full
                    .entry(d.date.clone())
                    .or_default()
                    .push(f as f64 / 1e6);
            }
        } else {
            d.estimated = true;
        }
        let span = day_span.entry(d.date.clone()).or_insert((cur.ts, cur.ts));
        span.0 = span.0.min(cur.ts);
        span.1 = span.1.max(cur.ts);
        if let Some(c) = cur.cycles {
            d.cycles = Some(d.cycles.map_or(c, |m: i64| m.max(c)));
        }

        if i == 0 {
            continue;
        }
        let prev = &rows[i - 1];
        let dt = cur.ts - prev.ts;
        if dt <= 0 {
            continue;
        }
        let hours = dt as f64 / 3600.0;
        let drain = drained_wh(prev, cur);
        let drained = drain.unwrap_or(0.0);
        let date = day_of(prev.ts);
        let d = days.entry(date.clone()).or_insert_with(|| DayAgg {
            date: date.clone(),
            ..Default::default()
        });

        // How much of this interval was spent suspended. The daemon measures
        // it directly; older rows only have the hole to go on.
        let seeded = prev.source == "upower" || cur.source == "upower";
        // upower only wrote a row when the percentage changed, so a quiet
        // stretch on AC looks exactly like a suspend -- and, if the last
        // status was Discharging, like hours on battery. Neither is knowable,
        // so a wide gap between seeded rows is attributed to nothing at all.
        if seeded && dt > INFERRED_GAP_S {
            for (day, h) in per_day(prev.ts, cur.ts) {
                days.entry(day.clone())
                    .or_insert_with(|| DayAgg {
                        date: day,
                        ..Default::default()
                    })
                    .unaccounted_h += h;
            }
            continue;
        }
        let powered_off =
            !prev.boot_id.is_empty() && !cur.boot_id.is_empty() && prev.boot_id != cur.boot_id;

        // A `start` row means the sampler just came up. On the same boot that
        // is the sampler having been down -- an upgrade, a restart, a crash --
        // and the hole says nothing about what the machine was doing. Reading
        // it as a suspend is how a 16-minute rebuild became "27 W standby".
        if cur.kind == "start" && !powered_off {
            for (day, h) in per_day(prev.ts, cur.ts) {
                days.entry(day.clone())
                    .or_insert_with(|| DayAgg {
                        date: day,
                        ..Default::default()
                    })
                    .unaccounted_h += h;
            }
            continue;
        }
        let measured = cur.slept_s.is_some();
        let slept = match cur.slept_s {
            Some(s) => s,
            None if dt > INFERRED_GAP_S && !seeded => dt,
            _ => 0,
        };
        let is_sleep = if measured {
            slept > MEASURED_SLEEP_S
        } else {
            slept > 0
        };

        if is_sleep || powered_off {
            // A machine that was off has no `slept_s` to report -- the daemon
            // it would have come from was not running -- so the whole interval
            // is the outage.
            let sleep_h = if powered_off {
                dt as f64 / 3600.0
            } else {
                slept.max(0) as f64 / 3600.0
            };
            let residency = hw_residency(prev, cur, slept);
            // Charge that arrived from the wall is not standby drain, and a
            // sleep that straddled a plug-in cannot be split. Say so.
            let on_ac = prev.ac != 0 || cur.ac != 0;
            // A pack that came out of the suspend *fuller* than it went in was
            // on a charger the endpoints did not admit to -- the old sampler
            // recorded "Discharging" straight through a 3-point gain. Reporting
            // that as 0.00 W standby is the flattering answer, not the true one.
            let gained = drain.map_or(false, |w| w < -0.05)
                || matches!((prev.capacity, cur.capacity), (Some(a), Some(b)) if b > a);
            let lost = match drain {
                Some(w) if !on_ac && !powered_off && !gained => Some(round2(w.max(0.0))),
                _ => None,
            };
            // The suspend ends at `cur`; place it there and work backwards, so
            // a sleep across midnight lands on both days in proportion.
            for (day, h) in per_day(cur.ts - (sleep_h * 3600.0) as i64, cur.ts) {
                let e = days.entry(day.clone()).or_insert_with(|| DayAgg {
                    date: day,
                    ..Default::default()
                });
                if powered_off {
                    e.off_h += h;
                } else {
                    e.asleep_h += h;
                    if let Some(w) = lost {
                        e.standby_wh += w * h / sleep_h.max(1e-9);
                        e.standby_h += h;
                    }
                }
            }
            sleeps.push(Sleep {
                start: prev.ts,
                end: cur.ts,
                hours: round2(sleep_h),
                drained_wh: lost,
                watts: match lost {
                    Some(w) if sleep_h > 0.0 => Some(round2(w / sleep_h)),
                    _ => None,
                },
                cap_from: prev.capacity,
                cap_to: cur.capacity,
                hw_residency: residency,
                on_ac,
                powered_off,
                exact: prev.kind == "presleep" && cur.kind == "resume",
            });
            // An interval that was mostly sleep tells us nothing about awake
            // draw, so it is not counted as time on battery.
            continue;
        }

        d.awake_h += hours;
        if prev.status == "Discharging" {
            d.battery_h += hours;
            if drained > 0.0 {
                d.drained_wh += drained;
                let hour = Local
                    .timestamp_opt(prev.ts, 0)
                    .single()
                    .map(|t| t.hour())
                    .unwrap_or(0);
                by_hour.entry(hour).or_default().add(drained, hours);
                if !prev.profile.is_empty() {
                    by_profile
                        .entry(prev.profile.clone())
                        .or_default()
                        .add(drained, hours);
                }
                if prev.elapsed_s.is_some() {
                    // Context columns only exist on rows this daemon wrote.
                    by_display
                        .entry(display_label(prev.displays_on, prev.external_displays))
                        .or_default()
                        .add(drained, hours);
                    if let Some(b) = prev.backlight {
                        by_backlight
                            .entry(backlight_label(b).to_string())
                            .or_default()
                            .add(drained, hours);
                    }
                    if let Some(c) = cur.cpu_busy {
                        by_cpu
                            .entry(cpu_label(c).to_string())
                            .or_default()
                            .add(drained, hours);
                    }
                }
                let ctx = day_ctx.entry(date.clone()).or_insert((0.0, 0.0, 0.0, 0.0));
                if let Some(b) = prev.backlight {
                    ctx.0 += b * hours;
                    ctx.1 += hours;
                }
                if let Some(c) = cur.cpu_busy {
                    ctx.2 += c * hours;
                    ctx.3 += hours;
                }
            }
        } else if (prev.status == "Charging" || prev.status == "Full") && drained < 0.0 {
            d.charged_wh += -drained;
        }
    }

    let mut out: Vec<DayAgg> = days.into_values().collect();
    for d in out.iter_mut() {
        d.avg_w = if d.battery_h > 0.05 {
            Some(round2(d.drained_wh / d.battery_h))
        } else {
            None
        };
        d.standby_w = if d.standby_h > 0.2 {
            Some(round2(d.standby_wh / d.standby_h))
        } else {
            None
        };
        if let Some(v) = day_full.get(&d.date) {
            let mut v = v.clone();
            v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            d.full_wh = Some(round2(v[v.len() / 2]));
        }
        if let Some((first, last)) = day_span.get(&d.date) {
            let observed = (last - first) as f64 / 3600.0;
            d.unaccounted_h = round2((observed - d.awake_h - d.asleep_h - d.off_h).max(0.0));
        }
        if let Some((bl, bh, cl, ch)) = day_ctx.get(&d.date) {
            if *bh > 0.0 {
                d.backlight = Some(round2(bl / bh));
            }
            if *ch > 0.0 {
                d.cpu_busy = Some(round2(cl / ch));
            }
        }
        d.awake_h = round2(d.awake_h);
        d.battery_h = round2(d.battery_h);
        d.asleep_h = round2(d.asleep_h);
        d.off_h = round2(d.off_h);
        d.drained_wh = round2(d.drained_wh);
        d.charged_wh = round2(d.charged_wh);
        d.standby_wh = round2(d.standby_wh);
        d.standby_h = round2(d.standby_h);
    }

    let hour_buckets = (0..24)
        .map(|h| {
            by_hour
                .get(&h)
                .map(|a| a.bucket(&format!("{h:02}")))
                .unwrap_or(Bucket {
                    label: format!("{h:02}"),
                    avg_w: None,
                    hours: 0.0,
                    wh: 0.0,
                })
        })
        .collect();
    let sort_by_hours = |m: BTreeMap<String, Accum>| {
        let mut v: Vec<Bucket> = m.iter().map(|(k, a)| a.bucket(k)).collect();
        v.sort_by(|a, b| {
            b.hours
                .partial_cmp(&a.hours)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        v
    };

    (
        out,
        sleeps,
        hour_buckets,
        sort_by_hours(by_profile),
        sort_by_hours(by_display),
        sort_by_hours(by_backlight),
        sort_by_hours(by_cpu),
    )
}

/// Per-day series for the intra-day charts, bucketed so the page stays small.
pub fn curves(rows: &[Sample], bucket_s: i64) -> BTreeMap<String, Curve> {
    let mut per_day: BTreeMap<String, BTreeMap<i64, (Option<i64>, Vec<f64>, Vec<f64>, u8)>> =
        BTreeMap::new();
    for r in rows {
        let slot = (minutes_of(r.ts) * 60.0) as i64 / bucket_s;
        let day = per_day.entry(day_of(r.ts)).or_default();
        let e = day.entry(slot).or_insert((None, Vec::new(), Vec::new(), 0));
        if r.capacity.is_some() {
            e.0 = r.capacity;
        }
        match r.status.as_str() {
            "Discharging" => {
                e.3 = 1;
                if let Some(p) = r.power_uw {
                    if p > 0 {
                        e.1.push(p as f64 / 1e6);
                    }
                }
            }
            "Charging" => e.3 = 2,
            "Full" | "Not charging" => e.3 = 3,
            _ => {}
        }
        if let Some(c) = r.cpu_busy {
            e.2.push(c);
        }
    }

    let mean = |v: &Vec<f64>| -> Option<f64> {
        if v.is_empty() {
            None
        } else {
            Some(round2(v.iter().sum::<f64>() / v.len() as f64))
        }
    };
    per_day
        .into_iter()
        .map(|(day, slots)| {
            let mut c = Curve::default();
            for (slot, (cap, w, cpu, state)) in slots {
                c.t.push(round2(slot as f64 * bucket_s as f64 / 60.0));
                c.cap.push(cap);
                c.w.push(mean(&w));
                c.cpu.push(mean(&cpu));
                c.state.push(state);
            }
            (day, c)
        })
        .collect()
}

/// Everything the window shows, in one call.
pub fn dashboard(store: &Store, days: i64, curve_days: i64) -> rusqlite::Result<Dashboard> {
    let now = crate::sysfs::now_unix();
    let since = now - days * 86_400;
    let rows = store.rows(since, i64::MAX)?;
    let (aggs, sleeps, by_hour, by_profile, by_display, by_backlight, by_cpu) = analyse(&rows);

    let curve_since = now - curve_days.min(days) * 86_400;
    let recent: Vec<Sample> = rows
        .iter()
        .filter(|r| r.ts >= curve_since)
        .cloned()
        .collect();
    let curve = curves(&recent, 120);

    let (samples, first, bytes) = store.stats()?;
    let seeded: i64 = store.conn.query_row(
        "SELECT COUNT(*) FROM samples WHERE source = 'upower' AND ts >= ?1",
        [since],
        |r| r.get(0),
    )?;

    Ok(Dashboard {
        generated: now,
        range_days: days,
        battery: rows.last().map(battery_from),
        days: aggs,
        curve,
        sleeps,
        by_hour,
        by_profile,
        by_display,
        by_backlight,
        by_cpu,
        top_procs: store.top_procs_on_battery(since, i64::MAX, 12)?,
        log: LogInfo {
            samples,
            since: first.map(day_of),
            bytes,
            seeded,
        },
    })
}

pub fn battery_from(s: &Sample) -> BatteryNow {
    let full_wh = s.full_uwh.map(|v| round2(v as f64 / 1e6));
    let design_wh = s.design_uwh.map(|v| round2(v as f64 / 1e6));
    BatteryNow {
        model: s.model.clone(),
        capacity: s.capacity,
        status: s.status.clone(),
        power_w: s.power_uw.map(|v| round2(v as f64 / 1e6)),
        profile: s.profile.clone(),
        full_wh,
        design_wh,
        health: match (full_wh, design_wh) {
            (Some(f), Some(d)) if d > 0.0 => Some(round2(100.0 * f / d)),
            _ => None,
        },
        cycles: s.cycles,
        runtime_h: match (s.status.as_str(), s.energy_uwh, s.power_uw) {
            ("Discharging", Some(e), Some(p)) if p > 0 => Some(round2(e as f64 / p as f64)),
            _ => None,
        },
        backlight: s.backlight,
        displays_on: s.displays_on,
        external_displays: s.external_displays,
        cpu_busy: s.cpu_busy,
        temp_c: s.temp_c,
    }
}
