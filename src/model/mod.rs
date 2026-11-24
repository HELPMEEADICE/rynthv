mod automation;
mod dictionary;
mod instrumental;
mod language;
mod performance;
mod phoneme;
pub mod pitch;
mod project;
mod retakes;
mod sv1_pitch;
mod time;
mod track_color;
mod voice;

pub use automation::*;
pub use dictionary::*;
pub use instrumental::*;
pub use language::*;
pub use performance::*;
pub use phoneme::*;
pub use pitch::{PitchControl, PitchCurve, PitchPoint};
pub use project::*;
pub use retakes::*;
pub use sv1_pitch::Sv1Pitch;
pub use time::*;
pub use track_color::*;
pub use voice::*;

use std::sync::atomic::{AtomicU64, Ordering};
pub type EntityId = u64;
pub const SVP_VERSION: i32 = 196;
pub fn new_id() -> EntityId {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}
pub type Extra = std::collections::BTreeMap<String, serde_json::Value>;
mod singer;
pub use singer::{inherits_singer_modes, singer_database};
