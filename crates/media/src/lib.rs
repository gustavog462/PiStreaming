//! Media engine: probe (ffprobe) y decisión de plan de reproducción.
pub mod decide;
pub mod probe;

pub use decide::{decide, Probe};
pub use probe::{probe, probe_from_json};
