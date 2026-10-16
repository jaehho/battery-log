// The window is the product; `battery-log status` is the same numbers as text,
// for when a terminal is already open.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod status;

use std::sync::Mutex;

use battery_log_core::sysfs::Reader;

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("status") => {
            if let Err(e) = status::print() {
                eprintln!("battery-log: {e}");
                std::process::exit(1);
            }
        }
        Some("dump") => {
            // The dashboard as JSON: what the window is drawing, for tests and
            // for working on the UI outside the shell.
            match battery_log_core::store::Store::open()
                .map_err(|e| e.to_string())
                .and_then(|s| {
                    battery_log_core::analysis::dashboard(&s, 30, 14).map_err(|e| e.to_string())
                }) {
                Ok(d) => {
                    let mut v = serde_json::to_value(&d).unwrap_or_default();
                    if let Some(obj) = v.as_object_mut() {
                        obj.insert(
                            "_palette".into(),
                            serde_json::to_value(battery_log_core::sysfs::desktop_palette())
                                .unwrap_or_default(),
                        );
                    }
                    println!("{}", serde_json::to_string(&v).unwrap_or_default());
                }
                Err(e) => {
                    eprintln!("battery-log: {e}");
                    std::process::exit(1);
                }
            }
        }
        Some("-h") | Some("--help") => {
            println!(
                "usage: battery-log [status]\n\n\
                 With no argument, opens the window.\n\
                 `status` prints the same numbers as text, `dump` prints them as JSON.\n\
                 The sampler is a separate daemon: battery-logd."
            );
        }
        Some(other) => {
            eprintln!("battery-log: unknown command {other}; try --help");
            std::process::exit(2);
        }
        None => run_app(),
    }
}

fn run_app() {
    tauri::Builder::default()
        .manage(commands::Live(Mutex::new(Reader::new())))
        .invoke_handler(tauri::generate_handler![
            commands::dashboard,
            commands::live,
            commands::day_procs,
            commands::stretch,
            commands::palette
        ])
        .run(tauri::generate_context!())
        .expect("failed to start battery-log");
}
