//! Media engine: probe (ffprobe) y decisión de plan de reproducción.
pub mod decide;

pub use decide::{decide, Probe};
