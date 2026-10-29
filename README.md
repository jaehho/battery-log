# battery-log

What the battery did, and why.

A sampler daemon writes one row a minute — faster while on battery, and once on
each side of every suspend — into a SQLite file. A desktop window reads it back
as charts: the day's charge and draw, the drain of every sleep, and what was on
screen and burning CPU while the energy went.

Nothing else on the desktop keeps this. UPower's own history is root-owned,
capped at a few days, records percent rather than energy, and has no query
surface; a battery applet shows the instant. This answers "what drained it
yesterday", "is the pack degrading", and "did that suspend actually suspend".

## Install

Arch, from the AUR:

```
paru -S battery-log-git
systemctl --user enable --now battery-logd
```

Or from a checkout, into `~/.local`, with no root and no AUR:

```
make install
systemctl --user enable --now battery-logd
```

That install is live: launches run the checkout's last build and rebuild the tree in the background for the next one, so edits land on the next launch or daemon restart without a package cycle; `BATTERY_LOG_NO_REBUILD=1` skips that rebuild for a script that calls `battery-log` often. A failed build never replaces a running binary — it lands in `~/.local/state/battery-log-live.log` instead. The two installs can coexist; the wrapper shadows the package in `PATH`, and either `make uninstall` or removing the package leaves the other working.

The first run adopts an existing `~/.local/state/battery.db` (the shell/Python
sampler this replaces) and, failing that, seeds itself from UPower's history so
the window has something to show on day one.

## Use

```
battery-log            the window
battery-log status     the same numbers as text
battery-log dump       the same numbers as JSON
battery-logd           the sampler and battery warnings (what the user service runs)
```

In the window: `1`–`4` switch tabs, `r` reloads, the range selector at the top
right sets how far back the aggregates go. Dragging across the draw chart
explains that stretch — its cost, brightness, profile, and the processes that
burned CPU inside exactly those minutes.

## What it records

Per sample, in `~/.local/state/battery-log/history.db`:

- **the pack** — status, percent, energy, full-charge and design capacity,
  power, voltage, cycle count, whether mains is attached, the power profile.
- **the context** — backlight, how many displays are lit and how many are
  external, CPU busy and deep-idle residency, load, package temperature, wifi
  and bluetooth state.
- **the time** — wall seconds since the last sample, how many of them the
  machine was *suspended* (the gap between `CLOCK_BOOTTIME` and
  `CLOCK_MONOTONIC`, so it is exact even with nothing watching), the kernel's
  hardware-sleep counter, and the boot id.
- **the cost** — once a minute on battery, the five processes that used the
  most CPU since the last snapshot.

## Battery warnings

The daemon also shows UPower's `WarningLevel` as one notification: *low*,
then *critical* in the same slot, then *hibernating*, closed when the charger
goes in. It has no thresholds of its own. They come from `UPower.conf`
(`PercentageLow`, `PercentageCritical`, `PercentageAction`), the same numbers
that drive UPower's critical action, so the warning and the hibernate cannot
disagree. It follows `PropertiesChanged` instead of polling, speaks only when
the level changes, and never suspends anything itself. `--no-notify` turns it
off.

## Why a daemon and not a timer

A `systemd` timer cannot do the two things that make the numbers trustworthy:

- **Suspend boundaries.** The daemon holds a logind delay inhibitor and samples
  on `PrepareForSleep`, so a sleep is one clean interval with a reading at each
  end. Inferring sleeps from holes in a timer's output turns a missed tick into
  a 20 W "standby drain".
- **An interval that follows the machine.** 20 seconds on battery, where the
  detail matters; 60 on wall power, where nothing moves.

Everything it reads is unprivileged: `/sys/class/power_supply`, `/sys/class/drm`,
`/sys/class/backlight`, `/sys/devices/system/cpu/cpuidle`, `/sys/power`, and
`/proc`. No root, no RAPL, no `powertop`.

## What the numbers refuse to say

Most of `analysis.rs` is about not answering. The failure mode of a battery
chart is a plausible wrong number, so an interval that cannot be read is
attributed to nothing and shows as *unaccounted*, and a value that cannot be
measured prints as a dash rather than a zero. The rules that cost the most to
learn, each pinned by a test in `crates/battery-log-core/tests/analysis.rs`:

- **`energy_now` is not a fixed ruler.** The firmware re-estimates
  `energy_full` as it learns the pack — this one moves about 6 Wh, nearly 9% —
  and rebases `energy_now` with it. Subtracting two raw readings across a
  recalibration invents watt-hours. Where the ruler moves, the state of charge
  is differenced instead. Over one 16-hour suspend that is the difference
  between "0.00 W" and the real 0.11 W.
- **A sleep on the charger has no standby number.** Neither does one the pack
  came out of fuller than it went in. Both used to report 0.00 W, which is the
  most flattering possible lie.
- **`total_hw_sleep` is not always a measurement.** It has advanced by exactly
  2^32 µs across a 71-minute suspend here, which reads as 101% residency.
  Prefer the per-suspend `last_hw_sleep`, and drop anything that claims more
  hardware sleep than there was sleep.
- **A suspend across midnight belongs to both days.** Charging all of it to the
  first is how one day in the log came back thirty-two hours long.
- **A hole is not a suspend.** A measured one has to exceed a minute; an
  inferred one has to exceed ten. A `start` row means the sampler was down, a
  changed `boot_id` means the machine was off, and a gap between UPower's
  seeded rows means nothing at all.

The window carries the same distinction: solid marks are measured, hatched ones
are estimated from the seeded rows UPower left behind.

## Layout

```
crates/battery-log-core/   sampling, storage, and the analysis
  src/sysfs.rs             reading the machine
  src/procs.rs             per-process CPU deltas
  src/store.rs             the SQLite log, and adopting the old one
  src/analysis.rs          rows -> the numbers the window draws
  src/daemon.rs            the loop, the sleep signals, the inhibitor
  src/notify.rs            UPower warning level -> one notification
  src/bin/battery-logd.rs  the sampler binary (no GUI linked)
src-tauri/                 the window: IPC commands and the text/JSON CLI
ui/                        the dashboard — static HTML, CSS and one JS file
scripts/                   the live wrapper `make install` lays down, and its test
packaging/                 AUR recipe, user service, launcher entry
```

`make test` covers the interval walk, which is where every number comes from
and whose failure mode is a plausible-looking wrong chart — plus the install
wrapper, whose failure mode is silently running nothing.

`make lint` runs clippy with warnings as errors, then `cargo fmt --check`. `make help` lists every target.
