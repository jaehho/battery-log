//! Reading the machine. Everything here is unprivileged sysfs and procfs.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::model::{Kind, Sample};

const PS: &str = "/sys/class/power_supply";

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn read(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn read_i64(path: impl AsRef<Path>) -> Option<i64> {
    read(path)?.parse().ok()
}

fn read_f64(path: impl AsRef<Path>) -> Option<f64> {
    read(path)?.parse().ok()
}

/// A pack that powers the machine, not a mouse or a headset -- peripherals
/// live in the same directory and are marked `scope=Device`.
fn system_batteries() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(dir) = fs::read_dir(PS) {
        for e in dir.flatten() {
            let p = e.path();
            if read(p.join("type")).as_deref() != Some("Battery") {
                continue;
            }
            if read(p.join("scope")).unwrap_or_else(|| "System".into()) != "System" {
                continue;
            }
            out.push(p);
        }
    }
    out.sort();
    out
}

/// Normalise a pack to energy units. Laptops report either energy (uWh/uW) or
/// charge (uAh/uA); the charge kind is converted with the live voltage so both
/// land in the same columns.
struct Pack {
    status: String,
    capacity: Option<i64>,
    energy_uwh: Option<i64>,
    full_uwh: Option<i64>,
    design_uwh: Option<i64>,
    power_uw: Option<i64>,
    voltage_uv: Option<i64>,
    cycles: Option<i64>,
    model: String,
}

fn read_pack(dir: &Path) -> Pack {
    let voltage = read_i64(dir.join("voltage_now"));
    let scale = voltage.unwrap_or(0) as f64 / 1e6;
    let energy = |e: &str, c: &str| -> Option<i64> {
        read_i64(dir.join(e)).or_else(|| {
            let charge = read_i64(dir.join(c))?;
            if scale > 0.0 {
                Some((charge as f64 * scale) as i64)
            } else {
                None
            }
        })
    };
    Pack {
        status: read(dir.join("status")).unwrap_or_else(|| "Unknown".into()),
        capacity: read_i64(dir.join("capacity")),
        energy_uwh: energy("energy_now", "charge_now"),
        full_uwh: energy("energy_full", "charge_full"),
        design_uwh: energy("energy_full_design", "charge_full_design"),
        power_uw: energy("power_now", "current_now"),
        voltage_uv: voltage,
        cycles: read_i64(dir.join("cycle_count")),
        model: read(dir.join("model_name"))
            .unwrap_or_else(|| dir.file_name().unwrap_or_default().to_string_lossy().into()),
    }
}

/// Whether a wall charger is attached.
///
/// Deliberately ignores the `ucsi-source-psy-*` nodes: they report online=1 for
/// a USB-C port with nothing useful on it while ADP1 says 0 and the pack
/// genuinely drains. The pack's own `status` is the tiebreaker everywhere it
/// matters.
fn mains_online() -> i64 {
    if let Ok(dir) = fs::read_dir(PS) {
        for e in dir.flatten() {
            let p = e.path();
            let name = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if name.starts_with("ucsi-") {
                continue;
            }
            if read(p.join("type")).as_deref() == Some("Mains")
                && read_i64(p.join("online")) == Some(1)
            {
                return 1;
            }
        }
    }
    0
}

fn backlight_percent() -> Option<f64> {
    let dir = fs::read_dir("/sys/class/backlight")
        .ok()?
        .flatten()
        .next()?
        .path();
    let now = read_f64(dir.join("brightness"))?;
    let max = read_f64(dir.join("max_brightness"))?;
    if max > 0.0 {
        Some((now / max * 100.0).round())
    } else {
        None
    }
}

/// (displays lit, of which external). A connector only counts when it is
/// connected *and* enabled: an unplugged HDMI port can sit at dpms=On forever.
fn displays() -> (i64, i64) {
    let (mut on, mut external) = (0, 0);
    if let Ok(dir) = fs::read_dir("/sys/class/drm") {
        for e in dir.flatten() {
            let p = e.path();
            let name = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if !name.contains('-') {
                continue; // cardN itself, not a connector
            }
            if read(p.join("status")).as_deref() != Some("connected") {
                continue;
            }
            if read(p.join("enabled")).as_deref() != Some("enabled") {
                continue;
            }
            if read(p.join("dpms")).as_deref() != Some("On") {
                continue;
            }
            on += 1;
            if !name.contains("eDP") && !name.contains("LVDS") {
                external += 1;
            }
        }
    }
    (on, external)
}

fn package_temp_c() -> Option<f64> {
    let dir = fs::read_dir("/sys/class/thermal").ok()?;
    let mut fallback = None;
    for e in dir.flatten() {
        let p = e.path();
        let kind = read(p.join("type")).unwrap_or_default();
        let temp = read_f64(p.join("temp")).map(|t| t / 1000.0);
        if kind == "x86_pkg_temp" {
            return temp;
        }
        if fallback.is_none() && kind == "acpitz" {
            fallback = temp;
        }
    }
    fallback
}

/// (wifi enabled, bluetooth enabled) from rfkill's soft-block state.
fn radios() -> (Option<i64>, Option<i64>) {
    let (mut wifi, mut bt) = (None, None);
    if let Ok(dir) = fs::read_dir("/sys/class/rfkill") {
        for e in dir.flatten() {
            let p = e.path();
            let blocked = read_i64(p.join("soft")).map(|s| i64::from(s == 0));
            match read(p.join("type")).as_deref() {
                Some("wlan") => wifi = wifi.or(blocked),
                Some("bluetooth") => bt = bt.or(blocked),
                _ => {}
            }
        }
    }
    (wifi, bt)
}

fn boot_id() -> String {
    read("/proc/sys/kernel/random/boot_id").unwrap_or_default()
}

/// Seconds since boot *including* time spent suspended (CLOCK_BOOTTIME).
fn uptime_s() -> f64 {
    read("/proc/uptime")
        .and_then(|s| s.split_whitespace().next().and_then(|v| v.parse().ok()))
        .unwrap_or(0.0)
}

/// (busy jiffies, total jiffies) across all CPUs.
fn cpu_jiffies() -> Option<(u64, u64)> {
    let stat = read("/proc/stat")?;
    let line = stat.lines().next()?;
    let vals: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|v| v.parse().ok())
        .collect();
    if vals.len() < 5 {
        return None;
    }
    let total: u64 = vals.iter().sum();
    let idle = vals[3] + vals[4]; // idle + iowait
    Some((total - idle, total))
}

/// Microseconds every CPU has spent in a C-state deeper than C1. Shallow
/// states keep the package awake; this is the number that says whether the
/// machine was really idling or merely looking idle.
fn deep_idle_us() -> Option<u64> {
    let mut total = 0u64;
    let mut seen = false;
    for cpu in fs::read_dir("/sys/devices/system/cpu").ok()?.flatten() {
        let idle = cpu.path().join("cpuidle");
        if !idle.is_dir() {
            continue;
        }
        for state in fs::read_dir(&idle).ok()?.flatten() {
            let p = state.path();
            let name = read(p.join("name")).unwrap_or_default();
            if name == "POLL" || name.starts_with("C1") {
                continue;
            }
            if let Some(t) = read_i64(p.join("time")) {
                total += t.max(0) as u64;
                seen = true;
            }
        }
    }
    if seen {
        Some(total)
    } else {
        None
    }
}

/// Kernel suspend bookkeeping: (cumulative hw-sleep us, last suspend's hw-sleep
/// us, successes, failures). `last_hw_sleep` is the honest one -- see
/// `Sample::last_hw_sleep_us`.
fn suspend_stats() -> (Option<i64>, Option<i64>, Option<i64>, Option<i64>) {
    let base = Path::new("/sys/power/suspend_stats");
    (
        read_i64(base.join("total_hw_sleep")),
        read_i64(base.join("last_hw_sleep")),
        read_i64(base.join("success")),
        read_i64(base.join("fail")),
    )
}

/// power-profiles-daemon's active profile, over the system bus.
fn power_profile(conn: Option<&zbus::blocking::Connection>) -> String {
    let conn = match conn {
        Some(c) => c,
        None => return String::new(),
    };
    // The daemon was renamed; ask the new name first and fall back to the old.
    for (service, path, iface) in [
        (
            "org.freedesktop.UPower.PowerProfiles",
            "/org/freedesktop/UPower/PowerProfiles",
            "org.freedesktop.UPower.PowerProfiles",
        ),
        (
            "net.hadess.PowerProfiles",
            "/net/hadess/PowerProfiles",
            "net.hadess.PowerProfiles",
        ),
    ] {
        let reply = conn.call_method(
            Some(service),
            path,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(iface, "ActiveProfile"),
        );
        if let Ok(msg) = reply {
            if let Ok(v) = msg.body().deserialize::<zbus::zvariant::Value>() {
                if let Ok(s) = String::try_from(v) {
                    return s;
                }
            }
        }
    }
    String::new()
}

/// Carries the counters that only mean something as a difference.
pub struct Reader {
    last_instant: Option<Instant>,
    last_uptime: f64,
    last_cpu: Option<(u64, u64)>,
    last_deep_idle: Option<u64>,
    conn: Option<zbus::blocking::Connection>,
}

impl Default for Reader {
    fn default() -> Self {
        Self::new()
    }
}

impl Reader {
    pub fn new() -> Self {
        Reader {
            last_instant: None,
            last_uptime: 0.0,
            last_cpu: None,
            last_deep_idle: None,
            // A missing system bus is not fatal -- it only costs the profile.
            conn: zbus::blocking::Connection::system().ok(),
        }
    }

    /// Read everything once. `None` means there is no system battery.
    pub fn sample(&mut self, kind: Kind) -> Option<Sample> {
        let packs: Vec<Pack> = system_batteries().iter().map(|p| read_pack(p)).collect();
        let packs: Vec<Pack> = packs
            .into_iter()
            .filter(|p| p.energy_uwh.is_some())
            .collect();
        if packs.is_empty() {
            return None;
        }
        let sum = |f: fn(&Pack) -> Option<i64>| -> Option<i64> {
            let vals: Vec<i64> = packs.iter().filter_map(f).collect();
            if vals.is_empty() {
                None
            } else {
                Some(vals.iter().sum())
            }
        };
        // One pack draining means the machine is on battery, so the most
        // "discharging" status across packs wins.
        let rank = |s: &str| match s {
            "Discharging" => 0,
            "Charging" => 1,
            "Not charging" => 2,
            "Full" => 3,
            _ => 4,
        };
        let status = packs
            .iter()
            .min_by_key(|p| rank(&p.status))
            .map(|p| p.status.clone())
            .unwrap_or_else(|| "Unknown".into());

        let energy = sum(|p| p.energy_uwh);
        let full = sum(|p| p.full_uwh);
        let capacity = if packs.len() > 1 {
            match (energy, full) {
                (Some(e), Some(f)) if f > 0 => Some((100 * e / f) as i64),
                _ => packs[0].capacity,
            }
        } else {
            packs[0].capacity
        };

        // Elapsed wall time, and how much of it the machine was suspended:
        // /proc/uptime counts suspend, a monotonic Instant does not.
        let uptime = uptime_s();
        let instant = Instant::now();
        let (elapsed_s, slept_s) = match (self.last_instant, self.last_uptime) {
            (Some(prev), prev_uptime) if prev_uptime > 0.0 => {
                let awake = instant.duration_since(prev).as_secs_f64();
                let wall = uptime - prev_uptime;
                (
                    Some(wall.round() as i64),
                    Some((wall - awake).max(0.0).round() as i64),
                )
            }
            _ => (None, None),
        };

        let cpu = cpu_jiffies();
        let cpu_busy = match (self.last_cpu, cpu) {
            (Some((b0, t0)), Some((b1, t1))) if t1 > t0 => {
                Some(((b1 - b0) as f64 / (t1 - t0) as f64 * 100.0 * 10.0).round() / 10.0)
            }
            _ => None,
        };
        let deep = deep_idle_us();
        let deep_idle = match (self.last_deep_idle, deep, elapsed_s) {
            (Some(d0), Some(d1), Some(secs)) if secs > 0 => {
                let cpus = std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(1) as f64;
                let share = (d1.saturating_sub(d0)) as f64 / (secs as f64 * 1e6 * cpus) * 100.0;
                Some((share.min(100.0) * 10.0).round() / 10.0)
            }
            _ => None,
        };

        let (displays_on, external_displays) = displays();
        let (wifi, bluetooth) = radios();
        let (hw_sleep_us, last_hw_sleep_us, suspend_ok, suspend_fail) = suspend_stats();

        self.last_instant = Some(instant);
        self.last_uptime = uptime;
        self.last_cpu = cpu;
        self.last_deep_idle = deep;

        Some(Sample {
            ts: now_unix(),
            kind: kind.as_str().to_string(),
            boot_id: boot_id(),
            status,
            capacity,
            energy_uwh: energy,
            full_uwh: full,
            design_uwh: sum(|p| p.design_uwh),
            power_uw: sum(|p| p.power_uw),
            voltage_uv: packs[0].voltage_uv,
            cycles: packs.iter().filter_map(|p| p.cycles).max(),
            ac: mains_online(),
            profile: power_profile(self.conn.as_ref()),
            model: packs[0].model.clone(),
            backlight: backlight_percent(),
            displays_on,
            external_displays,
            cpu_busy,
            deep_idle,
            load1: read("/proc/loadavg")
                .and_then(|s| s.split_whitespace().next().and_then(|v| v.parse().ok())),
            temp_c: package_temp_c(),
            wifi,
            bluetooth,
            elapsed_s,
            slept_s,
            hw_sleep_us,
            last_hw_sleep_us,
            suspend_ok,
            suspend_fail,
            source: "sysfs".to_string(),
        })
    }
}

/// The desktop's color palette (`theme-apply`'s source of truth), so the
/// window reskins with the rest of the machine. The fallback is the same
/// scheme, hard-coded, for a machine that has no palette file.
pub fn desktop_palette() -> std::collections::BTreeMap<String, String> {
    const FALLBACK: &[(&str, &str)] = &[
        ("base", "141c1e"),
        ("mantle", "101719"),
        ("crust", "0c1315"),
        ("text", "cfdadc"),
        ("subtext1", "b6c0c2"),
        ("subtext0", "9da7a9"),
        ("surface0", "20282a"),
        ("surface1", "2f3739"),
        ("surface2", "41494c"),
        ("overlay0", "565f62"),
        ("overlay1", "6d7679"),
        ("overlay2", "848e91"),
        ("blue", "3f7dae"),
        ("sapphire", "00898f"),
        ("teal", "118a7a"),
        ("green", "4f8751"),
        ("yellow", "8e7426"),
        ("peach", "a6653e"),
        ("maroon", "aa5d64"),
        ("red", "ab5e59"),
        ("mauve", "7c6bac"),
        ("lavender", "6074b3"),
        ("sky", "04869c"),
        ("pink", "956298"),
    ];
    let mut out: std::collections::BTreeMap<String, String> = FALLBACK
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config")
        });
    if let Some(text) = read(base.join("theme/palette")) {
        for line in text.lines() {
            if line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let v = v.trim();
                if v.len() == 6 && v.chars().all(|c| c.is_ascii_hexdigit()) {
                    out.insert(k.trim().to_string(), v.to_string());
                }
            }
        }
    }
    out
}
