//! battery-log: record what the battery did, and explain it.
//!
//! The crate is split so the daemon never links a GUI: `sysfs` and `procs`
//! read the machine, `store` keeps the log, `analysis` turns rows into the
//! numbers the window draws, `notify` mirrors UPower's warning level as a
//! notification, and `daemon` is the loop that ties them together.

pub mod analysis;
pub mod daemon;
pub mod model;
pub mod notify;
pub mod procs;
pub mod store;
pub mod sysfs;

pub use analysis::{dashboard, Dashboard};
pub use model::{Kind, ProcDelta, Sample};
pub use store::Store;
