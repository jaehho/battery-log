//! The sampler daemon. No GUI, no GTK, no webkit -- this is what runs all day.

use battery_log_core::daemon::{self, Config};

fn main() {
    let mut cfg = Config::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--battery-interval" => {
                cfg.on_battery_s = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(cfg.on_battery_s)
            }
            "--ac-interval" => {
                cfg.on_ac_s = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(cfg.on_ac_s)
            }
            "--no-procs" => cfg.top_procs = 0,
            "--no-notify" => cfg.notify = false,
            "--procs-interval" => {
                cfg.procs_every_s = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(cfg.procs_every_s)
            }
            "-h" | "--help" => {
                println!(
                    "usage: battery-logd [--battery-interval S] [--ac-interval S] [--procs-interval S] [--no-procs] [--no-notify]\n\n\
                     Samples the battery into ~/.local/state/battery-log/history.db,\n\
                     and shows UPower's low-battery warning level as a notification.\n\
                     `battery-log` is the window over that log."
                );
                return;
            }
            other => eprintln!("battery-logd: ignoring unknown argument {other}"),
        }
    }
    if let Err(e) = daemon::run(cfg) {
        eprintln!("battery-logd: {e}");
        std::process::exit(1);
    }
}
