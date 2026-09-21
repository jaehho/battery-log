//! Everything the window can ask for. Each command opens its own connection:
//! SQLite handles are not Send, and the queries are milliseconds.

use std::sync::Mutex;

use battery_log_core::analysis::{self, BatteryNow, Dashboard};
use battery_log_core::model::{Kind, ProcDelta};
use battery_log_core::store::Store;
use battery_log_core::sysfs::Reader;

/// The live reader is kept across calls so its counters (CPU busy, deep idle)
/// have a previous value to difference against.
pub struct Live(pub Mutex<Reader>);

fn open() -> Result<Store, String> {
    Store::open().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn dashboard(days: i64, curve_days: i64) -> Result<Dashboard, String> {
    let store = open()?;
    analysis::dashboard(&store, days.clamp(1, 365), curve_days.clamp(1, 60))
        .map_err(|e| e.to_string())
}

/// A fresh read of the hardware, for the header. Never touches the log.
#[tauri::command]
pub fn live(state: tauri::State<'_, Live>) -> Result<Option<BatteryNow>, String> {
    let mut reader = state.0.lock().map_err(|e| e.to_string())?;
    Ok(reader
        .sample(Kind::Tick)
        .map(|s| analysis::battery_from(&s)))
}

/// What burned the CPU on one local day, while on battery.
#[tauri::command]
pub fn day_procs(date: String, limit: i64) -> Result<Vec<ProcDelta>, String> {
    let store = open()?;
    let start = analysis::day_start(&date);
    store
        .top_procs_on_battery(start, start + 86_400, limit.clamp(1, 50))
        .map_err(|e| e.to_string())
}

/// The desktop palette, so the window is the same colors as everything else on
/// the machine. Falls back to the built-in scheme when there is no palette file.
#[tauri::command]
pub fn palette() -> std::collections::BTreeMap<String, String> {
    battery_log_core::sysfs::desktop_palette()
}
