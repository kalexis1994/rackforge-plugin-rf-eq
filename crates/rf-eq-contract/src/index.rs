//! Named parameter indexes.
//!
//! The engine and the packager both address parameters by number. Naming them
//! here — and asserting in tests that each name still points at the identifier
//! it claims — keeps a renumbering from quietly rewiring a knob.

pub const HPF_ENABLE: u32 = 0;
pub const HPF_FREQUENCY: u32 = 1;
pub const LOW_SHELF_FREQUENCY: u32 = 2;
pub const LOW_SHELF_GAIN: u32 = 3;
pub const PEAK1_FREQUENCY: u32 = 4;
pub const PEAK1_GAIN: u32 = 5;
pub const PEAK1_Q: u32 = 6;
pub const PEAK2_FREQUENCY: u32 = 7;
pub const PEAK2_GAIN: u32 = 8;
pub const PEAK2_Q: u32 = 9;
pub const HIGH_SHELF_FREQUENCY: u32 = 10;
pub const HIGH_SHELF_GAIN: u32 = 11;
pub const OUTPUT: u32 = 12;
pub const BYPASS: u32 = 13;
pub const PEAK3_FREQUENCY: u32 = 14;
pub const PEAK3_GAIN: u32 = 15;
pub const PEAK3_Q: u32 = 16;
pub const PEAK4_FREQUENCY: u32 = 17;
pub const PEAK4_GAIN: u32 = 18;
pub const PEAK4_Q: u32 = 19;
pub const LPF_ENABLE: u32 = 20;
pub const LPF_FREQUENCY: u32 = 21;
