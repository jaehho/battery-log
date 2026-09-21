//! The text summary. Same analysis as the window, no GUI.

use battery_log_core::analysis::{self, DayAgg};
use battery_log_core::store::Store;
use battery_log_core::sysfs::now_unix;

fn hm(hours: f64) -> String {
    let total = (hours * 60.0).round() as i64;
    format!("{}h{:02}m", total / 60, total % 60)
}

pub fn print() -> Result<(), Box<dyn std::error::Error>> {
    let store = Store::open()?;
    let d = analysis::dashboard(&store, 8, 1)?;
    if let Some(b) = &d.battery {
        let mut line = format!(
            "{}  {}%, {}",
            b.model,
            b.capacity.unwrap_or(0),
            b.status.to_lowercase()
        );
        if let Some(w) = b.power_w {
            line.push_str(&format!(", {w:.1} W"));
        }
        if let Some(h) = b.runtime_h {
            line.push_str(&format!(", {} left", hm(h)));
        }
        println!("{line}");
        // Health reads the most recent day's *median* full-charge capacity, not
        // whatever the firmware is showing this second -- it re-estimates all
        // day, and this pack swings about 6 Wh doing it.
        let full = d.days.iter().rev().find_map(|x| x.full_wh).or(b.full_wh);
        if let (Some(f), Some(des)) = (full, b.design_wh) {
            println!(
                "health   {:.1}%  ({f:.1} / {des:.1} Wh)   {} cycles",
                100.0 * f / des,
                b.cycles.unwrap_or(0)
            );
        }
    }

    let today = analysis::day_start(&analysis::local_day(now_unix()));
    let is_today = |d: &DayAgg| analysis::day_start(&d.date) == today;
    if let Some(t) = d.days.iter().find(|x| is_today(x)) {
        let avg = t
            .avg_w
            .map(|w| format!(", avg {w:.1} W"))
            .unwrap_or_default();
        println!(
            "today    {} on battery, {:.1} Wh used{avg}",
            hm(t.battery_h),
            t.drained_wh
        );
        if let Some(sw) = t.standby_w {
            println!(
                "         {} asleep, {:.2} Wh standby ({sw:.2} W)",
                hm(t.asleep_h),
                t.standby_wh
            );
        }
    }

    let week: Vec<&DayAgg> = d.days.iter().filter(|x| x.battery_h > 0.05).collect();
    let wh: f64 = week.iter().map(|x| x.drained_wh).sum();
    let hours: f64 = week.iter().map(|x| x.battery_h).sum();
    if hours > 0.0 {
        println!(
            "7 days   avg {:.1} W over {} on battery ({}/day), {wh:.0} Wh total",
            wh / hours,
            hm(hours),
            hm(hours / week.len().max(1) as f64)
        );
    }

    // Suspends are the reason the daemon exists; say how they went. Only the
    // sleeps with a readable drain go into the average -- one spent on AC
    // would otherwise pull it toward a flattering zero.
    let sleeps: Vec<_> = d
        .sleeps
        .iter()
        .filter(|s| !s.powered_off && s.hours > 0.1)
        .collect();
    let measured: Vec<_> = sleeps.iter().filter(|s| s.drained_wh.is_some()).collect();
    if !measured.is_empty() {
        let slept: f64 = measured.iter().map(|s| s.hours).sum();
        let lost: f64 = measured.iter().flat_map(|s| s.drained_wh).sum();
        let residency: Vec<f64> = sleeps.iter().filter_map(|s| s.hw_residency).collect();
        let mut line = format!(
            "standby  {:.2} W asleep ({} over {} of {} sleeps)",
            lost / slept.max(0.001),
            hm(slept),
            measured.len(),
            sleeps.len()
        );
        if !residency.is_empty() {
            line.push_str(&format!(
                ", {:.0}% in hardware sleep",
                100.0 * residency.iter().sum::<f64>() / residency.len() as f64
            ));
        }
        println!("{line}");
    }

    println!(
        "log      {} samples since {}, {:.1} MB",
        d.log.samples,
        d.log.since.unwrap_or_else(|| "?".into()),
        d.log.bytes as f64 / 1e6
    );
    Ok(())
}
