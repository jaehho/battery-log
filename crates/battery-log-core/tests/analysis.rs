//! The interval walk is where every number in the window comes from, and its
//! failure mode is silent: a wrong rule produces a plausible chart. These
//! fixtures pin the rules that were got wrong at least once.

use battery_log_core::analysis::{analyse, day_start};
use battery_log_core::model::Sample;

fn sample(ts: i64, status: &str, energy_wh: f64) -> Sample {
    Sample {
        ts,
        kind: "tick".into(),
        boot_id: "boot-a".into(),
        status: status.into(),
        capacity: Some((energy_wh / 69.0 * 100.0).round() as i64),
        energy_uwh: Some((energy_wh * 1e6) as i64),
        full_uwh: Some(69_000_000),
        design_uwh: Some(83_000_000),
        power_uw: Some(12_000_000),
        ac: i64::from(status != "Discharging"),
        profile: "balanced".into(),
        source: "sysfs".into(),
        elapsed_s: Some(60),
        slept_s: Some(0),
        displays_on: 1,
        backlight: Some(80.0),
        cpu_busy: Some(10.0),
        ..Default::default()
    }
}

/// A discharge at a known rate must come back as that rate.
#[test]
fn measures_the_obvious_discharge() {
    let mut rows = Vec::new();
    // 12 W for an hour: 12 Wh out of the pack, one sample a minute.
    for i in 0..=60 {
        rows.push(sample(
            1_000_000 + i * 60,
            "Discharging",
            60.0 - 0.2 * i as f64,
        ));
    }
    let (days, sleeps, _hour, profiles, _disp, _bl, _cpu) = analyse(&rows);
    assert_eq!(days.len(), 1);
    let d = &days[0];
    assert!(
        (d.battery_h - 1.0).abs() < 0.01,
        "battery_h = {}",
        d.battery_h
    );
    assert!(
        (d.drained_wh - 12.0).abs() < 0.05,
        "drained = {}",
        d.drained_wh
    );
    assert!((d.avg_w.unwrap() - 12.0).abs() < 0.1, "avg = {:?}", d.avg_w);
    assert!(sleeps.is_empty());
    assert_eq!(profiles[0].label, "balanced");
}

/// A measured suspend is standby, not time on battery, and its hardware-sleep
/// residency comes from the counter delta.
#[test]
fn a_measured_sleep_is_standby() {
    let before = {
        let mut s = sample(2_000_000, "Discharging", 50.0);
        s.kind = "presleep".into();
        s.hw_sleep_us = Some(0);
        s
    };
    let after = {
        // Four hours later, having lost 1 Wh, of which 3h were hardware sleep.
        let mut s = sample(2_000_000 + 4 * 3600, "Discharging", 49.0);
        s.kind = "resume".into();
        s.slept_s = Some(4 * 3600);
        s.elapsed_s = Some(4 * 3600);
        s.hw_sleep_us = Some(3 * 3600 * 1_000_000);
        s
    };
    let (days, sleeps, ..) = analyse(&[before, after]);
    assert_eq!(sleeps.len(), 1);
    let s = &sleeps[0];
    assert!(s.exact, "boundary samples should mark the sleep exact");
    assert!((s.hours - 4.0).abs() < 0.01);
    assert!((s.drained_wh.unwrap() - 1.0).abs() < 0.01);
    assert!(
        (s.hw_residency.unwrap() - 0.75).abs() < 0.01,
        "residency {:?}",
        s.hw_residency
    );
    assert!(days[0].battery_h < 0.01, "a suspend is not time on battery");
    // The fixture happens to straddle midnight, so read the total, not day one.
    let standby: f64 = days.iter().map(|d| d.standby_wh).sum();
    assert!((standby - 1.0).abs() < 0.01, "standby {standby}");
}

/// A hole of a few minutes with nothing to explain it is not a suspend. Older
/// rows guessed it was, and produced 20 W "standby drain" from a missed tick.
#[test]
fn a_short_unexplained_hole_is_not_a_sleep() {
    let mut a = sample(3_000_000, "Discharging", 40.0);
    let mut b = sample(3_000_000 + 300, "Discharging", 39.8);
    a.slept_s = None;
    a.elapsed_s = None;
    b.slept_s = None;
    b.elapsed_s = None;
    let (_days, sleeps, ..) = analyse(&[a, b]);
    assert!(
        sleeps.is_empty(),
        "a five-minute hole should not be called a suspend"
    );
}

/// upower's seeded rows only exist where the percentage moved, so a wide gap
/// between them says nothing at all -- not sleep, not time on battery.
#[test]
fn seeded_gaps_are_attributed_to_nothing() {
    let mut a = sample(4_000_000, "Discharging", 40.0);
    let mut b = sample(4_000_000 + 6 * 3600, "Discharging", 30.0);
    a.source = "upower".into();
    b.source = "upower".into();
    a.slept_s = None;
    b.slept_s = None;
    a.elapsed_s = None;
    b.elapsed_s = None;
    let (days, sleeps, ..) = analyse(&[a, b]);
    assert!(sleeps.is_empty());
    assert!(
        days.iter().all(|d| d.battery_h < 0.01),
        "seeded gap counted as battery time"
    );
    assert!(
        days.iter().all(|d| d.standby_wh < 0.01),
        "seeded gap counted as standby"
    );
}

/// A different boot id across a hole means the machine was off, which is a
/// different thing from a suspend and must not be averaged in with them.
#[test]
fn a_reboot_is_not_a_suspend() {
    let a = sample(5_000_000, "Discharging", 40.0);
    let mut b = sample(5_000_000 + 3600, "Discharging", 39.5);
    b.boot_id = "boot-b".into();
    b.slept_s = Some(0);
    let (days, sleeps, ..) = analyse(&[a, b]);
    assert_eq!(sleeps.len(), 1);
    assert!(sleeps[0].powered_off);
    assert!(days[0].off_h > 0.9, "off time = {}", days[0].off_h);
    assert!(
        days[0].standby_wh < 0.01,
        "a power-off is not standby drain"
    );
}

/// The sampler being restarted is not the machine being asleep. Reading the
/// hole as a suspend produced a "27 W standby drain" from a rebuild.
#[test]
fn a_sampler_restart_is_not_a_sleep() {
    let before = sample(6_000_000, "Discharging", 40.0);
    let mut after = sample(6_000_000 + 16 * 60, "Discharging", 32.9);
    after.kind = "start".into();
    after.slept_s = None;
    after.elapsed_s = None;
    let (days, sleeps, ..) = analyse(&[before, after]);
    assert!(
        sleeps.is_empty(),
        "a sampler restart should not be called a suspend"
    );
    assert!(
        days[0].standby_wh < 0.01,
        "standby = {}",
        days[0].standby_wh
    );
    assert!(
        days[0].battery_h < 0.01,
        "an unobserved hole is not time on battery"
    );
}

/// The pack re-estimates its own full-charge capacity while the machine sleeps,
/// and rebases `energy_now` with it. Subtracting the raw readings then reports
/// a suspend that *gained* 3.4 Wh -- which the old code clamped to "0.00 W
/// standby", the most flattering possible lie. Numbers taken from the real log
/// on 2026-09-08.
#[test]
fn a_recalibration_mid_sleep_does_not_erase_the_drain() {
    let before = {
        let mut s = sample(4_000_000, "Discharging", 65.04);
        s.kind = "presleep".into();
        s.full_uwh = Some(65_090_000);
        s.capacity = Some(100);
        s
    };
    let after = {
        let mut s = sample(4_000_000 + 58_247, "Discharging", 68.45);
        s.kind = "resume".into();
        s.full_uwh = Some(70_300_000); // the firmware moved the ruler
        s.capacity = Some(97);
        s.slept_s = Some(58_247);
        s
    };
    let (_days, sleeps, ..) = analyse(&[before, after]);
    let s = &sleeps[0];
    // 3% of a ~70 Wh pack over 16.2 h is about 0.11 W, not zero and not negative.
    let w = s.watts.expect("a sleep on battery has a knowable drain");
    assert!((w - 0.11).abs() < 0.02, "watts = {w}");
}

/// A suspend that ran on the charger says nothing about standby drain. The
/// honest answer is "not measurable", not "0.00 W" -- and it must not dilute
/// the day's standby average.
#[test]
fn a_sleep_on_ac_reports_nothing_rather_than_zero() {
    let before = {
        let mut s = sample(5_000_000, "Discharging", 53.35);
        s.kind = "presleep".into();
        s
    };
    let after = {
        let mut s = sample(5_000_000 + 4_267, "Charging", 53.39);
        s.kind = "resume".into();
        s.slept_s = Some(4_267);
        s
    };
    let (days, sleeps, ..) = analyse(&[before, after]);
    assert!(sleeps[0].on_ac);
    assert!(sleeps[0].watts.is_none(), "watts = {:?}", sleeps[0].watts);
    assert!(days[0].asleep_h > 1.0, "it was still a suspend");
    assert!(
        days[0].standby_w.is_none(),
        "and it must not be averaged in"
    );
}

/// `total_hw_sleep` on this firmware has jumped by exactly 2^32 us across a
/// 71-minute suspend, which reads as 101% of the sleep spent in hardware
/// sleep. Prefer the per-suspend counter; drop anything impossible.
#[test]
fn an_impossible_residency_is_dropped_not_clamped() {
    let mk = |total: i64, last: Option<i64>| {
        let mut s = sample(6_000_000, "Discharging", 50.0);
        s.hw_sleep_us = Some(total);
        s.last_hw_sleep_us = last;
        s
    };
    let mut before = mk(68_719_476_736, None);
    before.kind = "presleep".into();

    let bogus = {
        let mut s = mk(73_014_444_032, None); // +2^32 us over a 4267 s sleep
        s.ts = 6_000_000 + 4_267;
        s.kind = "resume".into();
        s.slept_s = Some(4_267);
        s
    };
    let (_d, sleeps, ..) = analyse(&[before.clone(), bogus]);
    assert!(
        sleeps[0].hw_residency.is_none(),
        "{:?}",
        sleeps[0].hw_residency
    );

    let good = {
        let mut s = mk(73_014_444_032, Some(698_143_532));
        s.ts = 6_000_000 + 4_267;
        s.kind = "resume".into();
        s.slept_s = Some(4_267);
        s
    };
    let (_d, sleeps, ..) = analyse(&[before, good]);
    let r = sleeps[0].hw_residency.expect("last_hw_sleep is usable");
    assert!((r - 0.16).abs() < 0.01, "residency {r}");
}

/// A suspend that starts before midnight belongs to both days. Charging the
/// whole thing to the first is how 2026-09-07 came back as 32 hours asleep.
#[test]
fn a_sleep_across_midnight_is_split_between_the_days() {
    // 22:00 local to 14:00 the next day: 2 h on the first day, 14 h on the second.
    let start = day_start("2026-03-11") + 22 * 3600;
    let before = {
        let mut s = sample(start, "Discharging", 60.0);
        s.kind = "presleep".into();
        s
    };
    let after = {
        let mut s = sample(start + 16 * 3600, "Discharging", 58.0);
        s.kind = "resume".into();
        s.slept_s = Some(16 * 3600);
        s
    };
    let (days, sleeps, ..) = analyse(&[before, after]);
    assert_eq!(sleeps.len(), 1, "still one suspend, not two");
    for d in &days {
        assert!(
            d.asleep_h <= 24.0,
            "{} claims {} h asleep",
            d.date,
            d.asleep_h
        );
    }
    let total: f64 = days.iter().map(|d| d.asleep_h).sum();
    assert!((total - 16.0).abs() < 0.01, "total {total}");
    let first = days.iter().find(|d| d.date == "2026-03-11").unwrap();
    assert!(
        (first.asleep_h - 2.0).abs() < 0.01,
        "first day {}",
        first.asleep_h
    );
}

/// The old sampler recorded "Discharging" straight through a gap in which the
/// pack went from 85% to 88%. Whatever that interval was, it was not 0.00 W of
/// standby drain.
#[test]
fn a_gap_that_gained_charge_is_not_zero_watt_standby() {
    let mut a = sample(7_000_000, "Discharging", 58.65);
    let mut b = sample(7_000_000 + 3_660, "Discharging", 60.72);
    a.capacity = Some(85);
    b.capacity = Some(88);
    for s in [&mut a, &mut b] {
        s.slept_s = None;
        s.elapsed_s = None;
    }
    let (days, sleeps, ..) = analyse(&[a, b]);
    assert_eq!(sleeps.len(), 1);
    assert!(sleeps[0].watts.is_none(), "watts = {:?}", sleeps[0].watts);
    assert!(days[0].standby_w.is_none());
}
