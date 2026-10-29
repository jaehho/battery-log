//! Where samples live: one SQLite file, WAL so the app can read it while the
//! daemon writes.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

use crate::model::{ProcDelta, Sample};

pub const RETENTION_DAYS: i64 = 365;

/// One `ALTER TABLE` per column added since the first release; see
/// `Store::open_at`.
const ADDED_COLUMNS: &[&str] = &["ALTER TABLE samples ADD COLUMN last_hw_sleep_us INTEGER"];

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS samples (
    ts                INTEGER PRIMARY KEY,   -- unix seconds
    kind              TEXT NOT NULL DEFAULT 'tick',
    boot_id           TEXT NOT NULL DEFAULT '',
    status            TEXT NOT NULL,
    capacity          INTEGER,
    energy_uwh        INTEGER,
    full_uwh          INTEGER,
    design_uwh        INTEGER,
    power_uw          INTEGER,
    voltage_uv        INTEGER,
    cycles            INTEGER,
    ac                INTEGER NOT NULL DEFAULT 0,
    profile           TEXT NOT NULL DEFAULT '',
    model             TEXT NOT NULL DEFAULT '',
    backlight         REAL,
    displays_on       INTEGER,
    external_displays INTEGER,
    cpu_busy          REAL,
    deep_idle         REAL,
    load1             REAL,
    temp_c            REAL,
    wifi              INTEGER,
    bluetooth         INTEGER,
    elapsed_s         INTEGER,
    slept_s           INTEGER,
    hw_sleep_us       INTEGER,
    last_hw_sleep_us  INTEGER,
    suspend_ok        INTEGER,
    suspend_fail      INTEGER,
    source            TEXT NOT NULL DEFAULT 'sysfs'
);
CREATE INDEX IF NOT EXISTS samples_ts ON samples (ts);

CREATE TABLE IF NOT EXISTS procs (
    ts    INTEGER NOT NULL,
    rank  INTEGER NOT NULL,
    comm  TEXT    NOT NULL,
    cpu_s REAL    NOT NULL,
    PRIMARY KEY (ts, rank)
);
"#;

pub fn state_dir() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/state")
        });
    base.join("battery-log")
}

pub fn db_path() -> PathBuf {
    state_dir().join("history.db")
}

/// The Python sampler's database, which this replaces. Its rows are worth
/// keeping: they carry the upower seed and the first weeks of history.
fn legacy_db_path() -> PathBuf {
    state_dir()
        .parent()
        .map(|p| p.join("battery.db"))
        .unwrap_or_default()
}

/// Every column of `samples`, in the order `read_sample` expects.
const SAMPLE_COLS: &str = "ts, kind, boot_id, status, capacity, energy_uwh, full_uwh, design_uwh,
    power_uw, voltage_uv, cycles, ac, profile, model, backlight, displays_on,
    external_displays, cpu_busy, deep_idle, load1, temp_c, wifi, bluetooth,
    elapsed_s, slept_s, hw_sleep_us, last_hw_sleep_us, suspend_ok,
    suspend_fail, source";

fn read_sample(r: &rusqlite::Row) -> rusqlite::Result<Sample> {
    Ok(Sample {
        ts: r.get(0)?,
        kind: r.get(1)?,
        boot_id: r.get(2)?,
        status: r.get(3)?,
        capacity: r.get(4)?,
        energy_uwh: r.get(5)?,
        full_uwh: r.get(6)?,
        design_uwh: r.get(7)?,
        power_uw: r.get(8)?,
        voltage_uv: r.get(9)?,
        cycles: r.get(10)?,
        ac: r.get(11)?,
        profile: r.get(12)?,
        model: r.get(13)?,
        backlight: r.get(14)?,
        displays_on: r.get::<_, Option<i64>>(15)?.unwrap_or(0),
        external_displays: r.get::<_, Option<i64>>(16)?.unwrap_or(0),
        cpu_busy: r.get(17)?,
        deep_idle: r.get(18)?,
        load1: r.get(19)?,
        temp_c: r.get(20)?,
        wifi: r.get(21)?,
        bluetooth: r.get(22)?,
        elapsed_s: r.get(23)?,
        slept_s: r.get(24)?,
        hw_sleep_us: r.get(25)?,
        last_hw_sleep_us: r.get(26)?,
        suspend_ok: r.get(27)?,
        suspend_fail: r.get(28)?,
        source: r.get(29)?,
    })
}

pub struct Store {
    pub conn: Connection,
}

impl Store {
    pub fn open() -> rusqlite::Result<Store> {
        Store::open_at(&db_path())
    }

    pub fn open_at(path: &Path) -> rusqlite::Result<Store> {
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)?;
        // Columns added after the first release. SQLite has no "add if
        // missing", and a duplicate-column error here is the success case.
        for ddl in ADDED_COLUMNS {
            let _ = conn.execute(ddl, []);
        }
        let store = Store { conn };
        store.adopt_legacy()?;
        Ok(store)
    }

    /// One-time import of the Python log. Columns the old schema never had
    /// stay NULL, and the analysis treats them as unknown rather than zero.
    fn adopt_legacy(&self) -> rusqlite::Result<()> {
        let has_rows: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM samples", [], |r| r.get(0))?;
        if has_rows > 0 {
            return Ok(());
        }
        let old = legacy_db_path();
        if !old.exists() {
            return Ok(());
        }
        self.conn.execute(
            "ATTACH DATABASE ?1 AS legacy",
            params![old.to_string_lossy()],
        )?;
        let copied = self.conn.execute(
            "INSERT OR IGNORE INTO samples
               (ts, kind, status, capacity, energy_uwh, full_uwh, design_uwh,
                power_uw, voltage_uv, cycles, ac, profile, source)
             SELECT ts, 'tick', status, capacity, energy_uwh, full_uwh, design_uwh,
                    power_uw, voltage_uv, cycles, ac, COALESCE(profile, ''),
                    COALESCE(source, 'sysfs')
             FROM legacy.samples",
            [],
        )?;
        self.conn.execute_batch("DETACH DATABASE legacy")?;
        eprintln!(
            "battery-log: adopted {copied} samples from {}",
            old.display()
        );
        Ok(())
    }

    pub fn insert(&self, s: &Sample, procs: &[ProcDelta]) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO samples
               (ts, kind, boot_id, status, capacity, energy_uwh, full_uwh, design_uwh,
                power_uw, voltage_uv, cycles, ac, profile, model, backlight, displays_on,
                external_displays, cpu_busy, deep_idle, load1, temp_c, wifi, bluetooth,
                elapsed_s, slept_s, hw_sleep_us, last_hw_sleep_us, suspend_ok,
                suspend_fail, source)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,
                     ?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30)",
            params![
                s.ts,
                s.kind,
                s.boot_id,
                s.status,
                s.capacity,
                s.energy_uwh,
                s.full_uwh,
                s.design_uwh,
                s.power_uw,
                s.voltage_uv,
                s.cycles,
                s.ac,
                s.profile,
                s.model,
                s.backlight,
                s.displays_on,
                s.external_displays,
                s.cpu_busy,
                s.deep_idle,
                s.load1,
                s.temp_c,
                s.wifi,
                s.bluetooth,
                s.elapsed_s,
                s.slept_s,
                s.hw_sleep_us,
                s.last_hw_sleep_us,
                s.suspend_ok,
                s.suspend_fail,
                s.source
            ],
        )?;
        for (i, p) in procs.iter().enumerate() {
            self.conn.execute(
                "INSERT OR REPLACE INTO procs (ts, rank, comm, cpu_s) VALUES (?1,?2,?3,?4)",
                params![s.ts, i as i64, p.comm, p.cpu_s],
            )?;
        }
        Ok(())
    }

    pub fn prune(&self, days: i64) -> rusqlite::Result<()> {
        let cutoff = crate::sysfs::now_unix() - days * 86_400;
        self.conn
            .execute("DELETE FROM samples WHERE ts < ?1", params![cutoff])?;
        self.conn
            .execute("DELETE FROM procs WHERE ts < ?1", params![cutoff])?;
        Ok(())
    }

    pub fn rows(&self, since: i64, until: i64) -> rusqlite::Result<Vec<Sample>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SAMPLE_COLS} FROM samples WHERE ts >= ?1 AND ts < ?2 ORDER BY ts"
        ))?;
        let rows = stmt.query_map(params![since, until], read_sample)?;
        rows.collect()
    }

    /// The last row strictly before `ts`. The interval that *enters* a window
    /// starts outside it, so reading a stretch needs this one row from before.
    pub fn row_before(&self, ts: i64) -> rusqlite::Result<Option<Sample>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SAMPLE_COLS} FROM samples WHERE ts < ?1 ORDER BY ts DESC LIMIT 1"
        ))?;
        stmt.query_row(params![ts], read_sample).optional()
    }

    /// The first row at or after `ts`, for the interval that leaves a window.
    pub fn row_after(&self, ts: i64) -> rusqlite::Result<Option<Sample>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SAMPLE_COLS} FROM samples WHERE ts >= ?1 ORDER BY ts LIMIT 1"
        ))?;
        stmt.query_row(params![ts], read_sample).optional()
    }

    /// (total samples, first timestamp, file size in bytes)
    pub fn stats(&self) -> rusqlite::Result<(i64, Option<i64>, u64)> {
        let (n, first): (i64, Option<i64>) =
            self.conn
                .query_row("SELECT COUNT(*), MIN(ts) FROM samples", [], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
        let size = fs::metadata(db_path()).map(|m| m.len()).unwrap_or(0);
        Ok((n, first, size))
    }

    /// CPU seconds per process name over a window, most expensive first.
    pub fn top_procs(
        &self,
        since: i64,
        until: i64,
        limit: i64,
    ) -> rusqlite::Result<Vec<ProcDelta>> {
        let mut stmt = self.conn.prepare(
            "SELECT comm, SUM(cpu_s) AS total FROM procs
             WHERE ts >= ?1 AND ts < ?2 GROUP BY comm ORDER BY total DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![since, until, limit], |r| {
            Ok(ProcDelta {
                comm: r.get(0)?,
                cpu_s: r.get(1)?,
            })
        })?;
        rows.collect()
    }
}

impl Store {
    /// The same, but only CPU burned while the machine was on battery -- the
    /// only case where "what drained it" is a meaningful question.
    pub fn top_procs_on_battery(
        &self,
        since: i64,
        until: i64,
        limit: i64,
    ) -> rusqlite::Result<Vec<ProcDelta>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.comm, SUM(p.cpu_s) AS total
             FROM procs p JOIN samples s ON s.ts = p.ts
             WHERE p.ts >= ?1 AND p.ts < ?2 AND s.status = 'Discharging'
             GROUP BY p.comm ORDER BY total DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![since, until, limit], |r| {
            Ok(ProcDelta {
                comm: r.get(0)?,
                cpu_s: r.get(1)?,
            })
        })?;
        rows.collect()
    }
}
